import { useEffect, useState } from 'react';
import { Bot, KeyRound, Plus } from 'lucide-react';
import { agentApi } from '@/api/agent';
import type { CreateProviderRequest, LlmProviderKind, ProviderRecord } from '@/api/types';
import { Field, Toggle } from '@/components/Field';
import { Modal } from '@/components/Modal';
import { Spinner } from '@/components/Spinner';
import { errorMessageKey, useI18n } from '@/i18n/I18nProvider';
import { cn } from '@/lib/cn';
import { useToast } from '@/state/ToastProvider';
import {
  BASE_URL_PLACEHOLDERS,
  MODEL_PLACEHOLDERS,
  PROVIDER_HINT_KEYS,
  PROVIDER_KINDS,
  PROVIDER_LABEL_KEYS,
} from './constants';

interface ProviderDialogProps {
  provider: ProviderRecord | null;
  onClose: () => void;
  onSaved: () => void;
}

/** Add/edit dialog for an LLM provider. API keys are write-only. */
export function ProviderDialog({ provider, onClose, onSaved }: ProviderDialogProps): React.JSX.Element {
  const { t } = useI18n();
  const toast = useToast();
  const [kind, setKind] = useState<LlmProviderKind>(provider?.kind ?? 'openai');
  const [label, setLabel] = useState(provider?.label ?? '');
  const [model, setModel] = useState(provider?.model ?? '');
  const [baseUrl, setBaseUrl] = useState(provider?.base_url ?? '');
  const [apiKey, setApiKey] = useState('');
  const [enabled, setEnabled] = useState(provider?.enabled ?? true);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    if (provider === null) return;
    setKind(provider.kind);
    setLabel(provider.label);
    setModel(provider.model);
    setBaseUrl(provider.base_url ?? '');
    setEnabled(provider.enabled);
  }, [provider]);

  const requiresBaseUrl = kind === 'custom';

  const submit = async (): Promise<void> => {
    setError(null);
    if (label.trim() === '' || model.trim() === '') {
      setError(t('errors.validation'));
      return;
    }
    if (requiresBaseUrl && baseUrl.trim() === '') {
      setError(t('agent.baseUrl'));
      return;
    }
    const payload: CreateProviderRequest = {
      kind,
      label: label.trim(),
      model: model.trim(),
      base_url: baseUrl.trim() === '' ? null : baseUrl.trim(),
      api_key: apiKey.trim() === '' ? null : apiKey.trim(),
      enabled,
    };
    setBusy(true);
    try {
      await agentApi.createProvider(payload);
      toast.push({ level: 'success', message: t('agent.toast.providerCreated') });
      onSaved();
    } catch (cause) {
      setError(t(errorMessageKey(cause)));
    } finally {
      setBusy(false);
    }
  };

  return (
    <Modal
      open
      onClose={busy ? () => undefined : onClose}
      locked={busy}
      size="lg"
      title={provider === null ? t('agent.providers.add') : t('agent.providers.add')}
      description={t('agent.providers.subtitle')}
      footer={
        <>
          <button type="button" className="btn btn-secondary" onClick={onClose} disabled={busy}>
            {t('common.cancel')}
          </button>
          <button type="button" className="btn btn-primary" onClick={() => void submit()} disabled={busy}>
            {busy ? <Spinner size={16} className="text-[#04121a]" /> : <Plus size={16} />}
            {busy ? t('common.saving') : t('common.save')}
          </button>
        </>
      }
    >
      <div className="space-y-4">
        {error !== null ? (
          <p role="alert" className="rounded-xl border border-coral-400/25 bg-coral-500/[0.08] px-3.5 py-2.5 text-[13px] text-coral-300">
            {error}
          </p>
        ) : null}

        <Field label={t('agent.providerKind')}>
          <div className="flex flex-wrap gap-1.5">
            {PROVIDER_KINDS.map((value) => (
              <button
                key={value}
                type="button"
                onClick={() => {
                  setKind(value);
                  if (label === '') setLabel(t(PROVIDER_LABEL_KEYS[value]));
                  if (value === 'opencode_go') {
                    if (baseUrl.trim() === '') setBaseUrl(BASE_URL_PLACEHOLDERS.opencode_go);
                    if (model.trim() === '') setModel(MODEL_PLACEHOLDERS.opencode_go);
                  }
                }}
                aria-pressed={kind === value}
                className={cn(
                  'inline-flex min-h-[40px] items-center gap-1.5 rounded-xl border px-3 text-[12.5px] transition',
                  kind === value
                    ? 'border-mint-400/35 bg-mint-400/[0.08] text-mint-200'
                    : 'border-white/[0.08] bg-white/[0.02] text-ink-400 hover:border-white/[0.16] hover:text-ink-200',
                )}
              >
                <Bot size={14} />
                {t(PROVIDER_LABEL_KEYS[value])}
              </button>
            ))}
          </div>
          <p className="mt-1.5 text-[11px] text-ink-500">{t(PROVIDER_HINT_KEYS[kind])}</p>
        </Field>

        <div className="grid gap-4 sm:grid-cols-2">
          <Field label={t('agent.label')} htmlFor="provider-label" required>
            <input
              id="provider-label"
              className="input"
              value={label}
              onChange={(event) => setLabel(event.target.value)}
              placeholder={t(PROVIDER_LABEL_KEYS[kind])}
            />
          </Field>
          <Field label={t('agent.model')} htmlFor="provider-model" required>
            <input
              id="provider-model"
              className="input input-mono"
              value={model}
              onChange={(event) => setModel(event.target.value)}
              placeholder={MODEL_PLACEHOLDERS[kind]}
              spellCheck={false}
            />
          </Field>
        </div>

        <Field
          label={t('agent.baseUrl')}
          htmlFor="provider-base-url"
          hint={requiresBaseUrl ? t(PROVIDER_HINT_KEYS[kind]) : t('common.optional')}
        >
          <input
            id="provider-base-url"
            className="input input-mono"
            value={baseUrl}
            onChange={(event) => setBaseUrl(event.target.value)}
            placeholder={BASE_URL_PLACEHOLDERS[kind]}
            spellCheck={false}
          />
        </Field>

        <Field
          label={t('agent.apiKey')}
          htmlFor="provider-api-key"
          hint={provider === null ? t('agent.apiKeyHint') : t('agent.apiKeyKeep')}
        >
          <div className="relative">
            <KeyRound
              size={15}
              className="pointer-events-none absolute left-3.5 top-1/2 -translate-y-1/2 text-ink-500"
            />
            <input
              id="provider-api-key"
              className="input pl-10"
              type="password"
              value={apiKey}
              onChange={(event) => setApiKey(event.target.value)}
              placeholder="sk-…"
              autoComplete="off"
            />
          </div>
        </Field>

        <Toggle
          checked={enabled}
          onChange={setEnabled}
          label={t('agent.providers.enabled')}
          description={t('agent.providers.subtitle')}
        />

        {provider !== null ? (
          <div className="flex items-center justify-between rounded-xl border border-white/[0.06] bg-white/[0.02] px-3.5 py-2.5">
            <span className="inline-flex items-center gap-1 text-[11px] text-ink-500">
              <KeyRound size={12} /> {t('agent.providers.noKey')}
            </span>
            <span className="font-mono text-[10px] text-ink-600">{provider.api_key_secret}</span>
          </div>
        ) : null}
      </div>
    </Modal>
  );
}
