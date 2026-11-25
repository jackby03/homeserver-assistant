import { useState } from 'react';
import { useSearchParams } from 'react-router-dom';
import { Bot, Check, KeyRound, Plus, RefreshCw, Trash2, Zap } from 'lucide-react';
import { agentApi } from '@/api/agent';
import { profilesApi } from '@/api/profiles';
import type { Profile, ProviderRecord, ToolDefinition } from '@/api/types';
import { ConfirmDialog } from '@/components/ConfirmDialog';
import { EmptyState } from '@/components/EmptyState';
import { PageHeader } from '@/components/PageHeader';
import { Spinner } from '@/components/Spinner';
import { StatusPill } from '@/components/StatusPill';
import { errorMessageKey, useI18n } from '@/i18n/I18nProvider';
import { cn } from '@/lib/cn';
import { useResource } from '@/lib/use-resource';
import { useToast } from '@/state/ToastProvider';
import { ChatTester } from './agent/ChatTester';
import { ProviderDialog } from './agent/ProviderDialog';
import { ToolsSection } from './agent/ToolsSection';
import { VoiceSection } from './agent/VoiceSection';
import { WorkflowsSection } from './agent/WorkflowsSection';
import { PROVIDER_LABEL_KEYS } from './agent/constants';

export function AgentPage(): React.JSX.Element {
  const { t } = useI18n();
  const toast = useToast();
  const [params] = useSearchParams();
  const [refreshToken, setRefreshToken] = useState(0);
  const [dialogOpen, setDialogOpen] = useState(params.get('addProvider') === '1');
  const [deleteTarget, setDeleteTarget] = useState<ProviderRecord | null>(null);
  const [deleting, setDeleting] = useState(false);
  const [activatingId, setActivatingId] = useState<string | null>(null);
  const [testingId, setTestingId] = useState<string | null>(null);

  const providers = useResource(() => agentApi.providers(), [refreshToken]);
  const tools = useResource(() => agentApi.tools(), [refreshToken]);
  const profiles = useResource(() => profilesApi.list(), [refreshToken]);

  const refresh = (): void => setRefreshToken((value) => value + 1);
  const providerList = providers.data?.providers ?? [];
  const toolList: ToolDefinition[] = tools.data ?? [];
  const profileList: Profile[] = profiles.data ?? [];
  const activeProvider = providerList.find((provider) => provider.enabled) ?? null;

  const activate = async (provider: ProviderRecord): Promise<void> => {
    setActivatingId(provider.id);
    try {
      await agentApi.activateProvider(provider.id);
      toast.push({ level: 'success', message: t('agent.toast.providerActivated') });
      refresh();
    } catch (cause) {
      toast.push({ level: 'error', title: t('toast.error'), message: t(errorMessageKey(cause)) });
    } finally {
      setActivatingId(null);
    }
  };

  const testProvider = async (provider: ProviderRecord): Promise<void> => {
    setTestingId(provider.id);
    try {
      const result = await agentApi.testProvider(provider.id);
      if (result.ok) {
        toast.push({
          level: 'success',
          title: t('agent.providers.testOk'),
          message: `${result.model ?? provider.model}: ${result.reply ?? ''}`,
        });
      } else {
        toast.push({
          level: 'error',
          title: t('agent.providers.testFailed'),
          message: result.error ?? t('errors.generic'),
        });
      }
    } catch (cause) {
      toast.push({ level: 'error', title: t('toast.error'), message: t(errorMessageKey(cause)) });
    } finally {
      setTestingId(null);
    }
  };

  const removeProvider = async (): Promise<void> => {
    if (deleteTarget === null) return;
    setDeleting(true);
    try {
      await agentApi.deleteProvider(deleteTarget.id);
      toast.push({ level: 'success', message: t('agent.toast.providerDeleted') });
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
        title={t('agent.title')}
        subtitle={t('agent.subtitle')}
        actions={
          <>
            <button type="button" className="btn btn-secondary btn-sm" onClick={refresh} disabled={providers.refreshing}>
              <RefreshCw size={14} className={providers.refreshing ? 'animate-spin' : undefined} />
              {t('common.refresh')}
            </button>
            <button type="button" className="btn btn-primary btn-sm" onClick={() => setDialogOpen(true)}>
              <Plus size={15} />
              {t('agent.providers.add')}
            </button>
          </>
        }
      />

      <VoiceSection />

      {/* Providers ------------------------------------------------------------ */}
      <section className="space-y-3">
        <div className="flex flex-wrap items-center justify-between gap-2">
          <div className="flex items-center gap-2">
            <h2 className="font-display text-[17px] font-medium text-ink-100">{t('agent.providers.title')}</h2>
            {activeProvider !== null ? (
              <StatusPill label={t('agent.providers.active')} tone="success" pulse />
            ) : null}
          </div>
          <span className="text-[11px] text-ink-500">{providerList.length}</span>
        </div>

        {providers.error !== null && providerList.length === 0 ? (
          <div className="rounded-2xl border border-coral-400/25 bg-coral-500/[0.07] px-4 py-3 text-sm text-coral-200">
            {t(errorMessageKey(providers.error))}
          </div>
        ) : providers.loading && providerList.length === 0 ? (
          <div className="grid gap-3 sm:grid-cols-2 xl:grid-cols-3">
            {Array.from({ length: 2 }).map((_, index) => (
              <div key={index} className="skeleton h-40 w-full rounded-2xl" />
            ))}
          </div>
        ) : providerList.length === 0 ? (
          <EmptyState
            icon={<Bot size={20} />}
            title={t('agent.providers.empty')}
            body={t('agent.providers.subtitle')}
            action={
              <button type="button" className="btn btn-primary btn-sm" onClick={() => setDialogOpen(true)}>
                <Plus size={15} />
                {t('agent.providers.add')}
              </button>
            }
          />
        ) : (
          <div className="grid gap-3 sm:grid-cols-2 xl:grid-cols-3">
            {providerList.map((provider) => {
              const isActive = activeProvider?.id === provider.id;
              return (
                <article
                  key={provider.id}
                  className={cn(
                    'glass glass-hover relative overflow-hidden p-5',
                    isActive && 'border-mint-400/25',
                  )}
                >
                  {isActive ? (
                    <div
                      aria-hidden="true"
                      className="pointer-events-none absolute -right-14 -top-14 h-36 w-36 rounded-full bg-mint-400/[0.08] blur-3xl"
                    />
                  ) : null}
                  <div className="relative flex items-start justify-between gap-3">
                    <div className="flex min-w-0 items-center gap-3">
                      <span className="grid h-10 w-10 shrink-0 place-items-center rounded-xl border border-mint-400/15 bg-mint-400/[0.06] text-mint-300">
                        <Bot size={18} />
                      </span>
                      <div className="min-w-0">
                        <h3 className="truncate text-[13.5px] font-medium text-ink-50">{provider.label}</h3>
                        <p className="truncate text-[11px] text-ink-500">
                          {t(PROVIDER_LABEL_KEYS[provider.kind])}
                        </p>
                      </div>
                    </div>
                    {isActive ? (
                      <span className="chip shrink-0 border-mint-400/25 bg-mint-400/[0.08] text-mint-300">
                        <Check size={11} />
                        {t('agent.providers.active')}
                      </span>
                    ) : (
                      <StatusPill
                        label={provider.enabled ? t('common.enabled') : t('common.disabled')}
                        tone={provider.enabled ? 'neutral' : 'neutral'}
                      />
                    )}
                  </div>

                  <div className="relative mt-3 space-y-1.5">
                    <p className="flex items-center gap-2 font-mono text-[11.5px] text-ink-300">
                      <Zap size={12} className="text-ink-500" />
                      {provider.model}
                    </p>
                    {provider.base_url !== null ? (
                      <p className="truncate font-mono text-[10.5px] text-ink-500">{provider.base_url}</p>
                    ) : null}
                    <p className="flex items-center gap-1.5 text-[10.5px] text-ink-600">
                      <KeyRound size={11} />
                      {provider.api_key_secret}
                    </p>
                  </div>

                  <div className="relative mt-4 flex items-center justify-between gap-2 border-t border-white/[0.05] pt-3">
                    <div className="flex items-center gap-2">
                      <button
                        type="button"
                        className="btn btn-secondary btn-sm"
                        onClick={() => void testProvider(provider)}
                        disabled={testingId !== null}
                        title={t('agent.providers.test')}
                      >
                        {testingId === provider.id ? <Spinner size={13} /> : <Zap size={13} />}
                        {t('agent.providers.test')}
                      </button>
                      <button
                        type="button"
                        className={cn('btn btn-sm', isActive ? 'btn-secondary' : 'btn-primary')}
                        onClick={() => void activate(provider)}
                        disabled={isActive || activatingId !== null}
                      >
                        {activatingId === provider.id ? <Spinner size={13} className="text-[#04121a]" /> : null}
                        {t('agent.providers.activate')}
                      </button>
                    </div>
                    <button
                      type="button"
                      className="btn btn-ghost btn-icon text-ink-500 hover:bg-coral-500/10 hover:text-coral-300"
                      onClick={() => setDeleteTarget(provider)}
                      aria-label={t('agent.providers.delete')}
                    >
                      <Trash2 size={15} />
                    </button>
                  </div>
                </article>
              );
            })}
          </div>
        )}
      </section>

      {/* Workflows + chat ------------------------------------------------------ */}
      <div className="grid items-start gap-4 xl:grid-cols-2">
        <WorkflowsSection
          profiles={profileList}
          tools={toolList}
          startWithEditor={params.get('newWorkflow') === '1'}
        />
        <ChatTester profiles={profileList} providers={providerList} />
      </div>

      <ToolsSection tools={toolList} loading={tools.loading} />

      {dialogOpen ? (
        <ProviderDialog
          provider={null}
          onClose={() => setDialogOpen(false)}
          onSaved={() => {
            setDialogOpen(false);
            refresh();
          }}
        />
      ) : null}

      <ConfirmDialog
        open={deleteTarget !== null}
        onClose={() => setDeleteTarget(null)}
        onConfirm={() => void removeProvider()}
        busy={deleting}
        title={t('agent.providers.confirmDelete.title', { name: deleteTarget?.label ?? '' })}
        message={t('agent.providers.confirmDelete.body')}
        confirmLabel={t('common.delete')}
      />
    </div>
  );
}
